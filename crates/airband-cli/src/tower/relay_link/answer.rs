//! A radio's sealed request, opened, checked and run through the same routes the local port serves, and its
//! answer sealed: one pulled from the relay and one that came straight are answered by the same code.
use super::super::{PRESENCE_HEADER, VIA_HEADER, log, now_ms};
use super::Serve;
use super::pull::Answering;
use super::said::RelaySaid;
use airband_core::radio::wire::Refusal;
use airband_core::relay::{Answer, Call, FRESH_MS, Route, fresh};
use airband_io::http::Ask;
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt;

/// Why a request too far from this computer's clock is refused, said from the limit itself.
fn wrong_clock() -> String {
    format!("the clocks of your phone and your computer are more than {} minutes apart", FRESH_MS / 60_000)
}

/// What a radio is told when whether its request was seen before cannot be read: it is not run, since it could
/// be a second time.
fn unchecked() -> Refusal {
    Refusal::NotNow { reason: "This computer could not check its records just now. It is sent again in a moment.".into() }
}

/// Whether a request was seen before.
enum Seen {
    First,
    Again,
}

impl Serve {
    /// Is this the first time a request of this id has come? Each is answered once: a copy of one, sent again by
    /// someone who saw it go by, is not, after a restart too. Ids are remembered for as long as a request counts
    /// as fresh. An error is said in the log and to the radio, and the request is not run.
    fn seen(&self, call: &Call, now: u64, via: &str) -> Result<Seen, (u16, Value)> {
        match self.app.store.first_time(&call.id, now, FRESH_MS * 2) {
            Ok(true) => Ok(Seen::First),
            Ok(false) => Ok(Seen::Again),
            Err(why) => {
                log(format!("database: whether a request from {} was seen before could not be read ({why})", self.tag));
                Err(self.refused(call, via, unchecked()))
            }
        }
    }

    /// A request that came straight to the tower: opened, checked and run as one from the relay is, and its
    /// answer sealed. Nothing for one this pairing's key does not open, that is stale, or that was seen before.
    pub(super) async fn answer_here(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        let call = self.keys.open_call(sealed)?;
        let now = now_ms();
        let via = "his own network";
        let (status, body) = if !fresh(call.t, now) {
            self.refused(&call, via, Refusal::told(wrong_clock()))
        } else {
            match self.seen(&call, now, via) {
                Ok(Seen::First) => self.dispatch(&call, via, true).await,
                Ok(Seen::Again) => return None,
                Err(refused) => refused,
            }
        };
        Some(self.keys.seal_answer(&Answer { id: call.id, status, body }))
    }

    /// Answer a request on a task of its own (`next` is held for 25 s, and other requests must get through
    /// meanwhile). However the task ends, a panic included, its guard tells the pulls it is no longer answered for.
    pub(super) fn answer_apart(&self, request_id: String, kept: bool, handing: Option<airband_core::relay::Handing>, sealed: Vec<u8>) {
        let guard = Answering { pulling: self.pulling.clone(), handing, delivered: false, turns: self.turns.clone() };
        let me = self.clone();
        drop(self.handle.spawn(async move {
            // the whole guard goes with the task, so it ends when the task does
            let mut guard = guard;
            guard.delivered = me.answer(request_id, kept, sealed).await;
        }));
    }

    /// Whether the relay has the answer (or, for a request that is not answered, that nothing is owed it).
    async fn answer(self, request_id: String, kept: bool, sealed: Vec<u8>) -> bool {
        let call = match self.keys.open_request(&sealed) {
            Some(Ok(call)) => Some(call),
            // opened with this pairing's key, in a version this tower cannot read: answered so, not dropped
            Some(Err(unread)) => {
                let no = Refusal::OtherVersion { speaks: airband_core::radio::SCHEMA.into() };
                let sealed = self.keys.seal_answer_for_relay(&Answer { id: unread.id, status: no.status(), body: serde_json::to_value(&no).unwrap_or_default() });
                return self.send_back(&request_id, kept, Some(sealed)).await;
            }
            None => None,
        };
        let Some(call) = call else {
            log(format!("dropped a request on {}'s channel that its key does not open", self.tag));
            // one the relay was keeping has to be taken off its hands all the same, or it would be handed over
            // for ever and hold up everything kept behind it
            return !kept || self.send_back(&request_id, kept, None).await;
        };
        let now = now_ms();
        // A message the relay kept while this tower was away is old on purpose, and may be handed over twice.
        // Only something he said is taken that way, and only with an id: the tower takes each id once, for good,
        // so neither its age nor a second copy can make it happen again.
        let (status, body) = if kept {
            self.kept(&call, now).await
        } else if !fresh(call.t, now) {
            // answered, not dropped, so a phone with a wrong clock is told why nothing works
            self.refused(&call, "the relay", Refusal::told(wrong_clock()))
        } else {
            match self.seen(&call, now, "the relay") {
                Ok(Seen::First) => self.dispatch(&call, "the relay", true).await,
                // its first copy is being answered; an answer to this one would reach the radio in its place
                Ok(Seen::Again) => {
                    log(format!("dropped a replayed request on {}'s channel", self.tag));
                    return true;
                }
                Err(refused) => refused,
            }
        };
        let sealed = self.keys.seal_answer_for_relay(&Answer { id: call.id, status, body });
        self.send_back(&request_id, kept, Some(sealed)).await
    }

    /// A request the relay kept while the tower was away.
    async fn kept(&self, call: &Call, now: u64) -> (u16, Value) {
        let via = "the relay, kept while the tower was away";
        // something he said, read as what it is: its id, as the tower takes it
        let said: Option<String> = (call.method == airband_core::radio::wire::Method::Post && call.path == <airband_core::radio::wire::at::Utterance as airband_core::radio::wire::Endpoint>::PATH).then(|| serde_json::from_value::<airband_core::radio::Utterance>(call.body.clone()).ok()).flatten().map(|u| u.id).filter(|id| !id.is_empty());
        if said.is_some() && now.saturating_sub(call.t) <= airband_core::relay::HELD_FOR_MS {
            log(format!("{} said this while the tower was away", self.tag));
            return self.dispatch(call, via, false).await;
        }
        // Nobody is waiting on this answer. Something he said that came too late is told to his radio, in its
        // feed, under the id it was said with: he was told it would get here.
        if let Some(id) = &said {
            self.app.change().settle(&self.keys.channel, id, airband_core::radio::Outcome::Rejected { why: airband_core::radio::Rejected::TooOld });
        }
        self.refused(call, via, Refusal::BadAsk { reason: "only something he said, sent in the last week, is taken late".into() })
    }

    /// Give the relay the answer to a request (or, with none, take a kept message off its hands). An answer
    /// that does not get there is written down: the radio that asked is still waiting for it. `kept`: the
    /// request was one the relay kept while the tower was away, said back to it as such. One of those whose
    /// answer did not get there is handed over again by the relay once the work on it ends ([`Answering`]).
    /// Whether the relay has it.
    async fn send_back(&self, request_id: &str, kept: bool, sealed: Option<Vec<u8>>) -> bool {
        let Some(relay) = &self.relay else { return false };
        let url = self.keys.to(relay, Route::Reply(request_id));
        let auth = self.keys.as_tower();
        // one agent for every answer on this channel: its connection to the relay is kept, not made again each time
        let agent = self.replies.clone();
        let sent = tokio::task::spawn_blocking(move || {
            let req = Ask::post(url).header("authorization", auth);
            let req = if kept { req.header(airband_core::relay::KEPT_HEADER, "1") } else { req };
            let req = match sealed {
                Some(sealed) => req.bytes("application/octet-stream", sealed),
                None => req,
            };
            agent.send(req).map(|r| RelaySaid::of(r.status().as_u16())).map_err(|e| e.to_string())
        })
        .await;
        match sent {
            Ok(Ok(RelaySaid::Nothing | RelaySaid::Done)) => {
                if self.replies_failing.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    log(format!("answers for {} reach {relay} again", self.tag));
                }
                true
            }
            // the radio gave up waiting before the answer was ready: it asks again, and nothing is lost
            Ok(Ok(RelaySaid::Closed)) => {
                log(format!("an answer for {} came after its radio stopped waiting (request {request_id})", self.tag));
                true
            }
            failed => {
                // said once while answers keep failing, not for every one: a kept answer that keeps failing is
                // handed over again and again
                if !self.replies_failing.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    log(match failed {
                        Ok(Ok(said)) => format!("{relay} answered {said:?} to an answer for {} (request {request_id}); said again once they get there", self.tag),
                        Ok(Err(e)) => format!("an answer for {} did not reach {relay} (request {request_id}): {e}; said again once they get there", self.tag),
                        Err(_) => format!("an answer for {} was not sent: its sending failed (request {request_id}); said again once they get there", self.tag),
                    });
                }
                false
            }
        }
    }

    /// A request the tower will not run, and the line that says so: which pairing, how it came, and why.
    fn refused(&self, call: &Call, via: &str, no: Refusal) -> (u16, Value) {
        log(format!("refused a request from {} by {via} ({} {}, {}): {}", self.tag, call.method, call.path.split('?').next().unwrap_or_default(), no.status(), no.reason().unwrap_or("no reason given")));
        (no.status(), serde_json::to_value(&no).unwrap_or_default())
    }

    /// Run a radio's request through the same routes the local port serves. Only the radio's routes: a paired
    /// phone cannot post agent events or read the status page.
    /// `now`: the request was made just now, so where its radio said it stands is where it stands. One the relay
    /// kept says where the radio stood when he said it, which is not news.
    async fn dispatch(&self, call: &Call, via: &str, now: bool) -> (u16, Value) {
        if !call.path.starts_with(airband_core::radio::PATH) || !matches!(call.method, airband_core::radio::wire::Method::Get | airband_core::radio::wire::Method::Post) {
            return self.refused(call, via, Refusal::NotAllowed);
        }
        let body = if call.body.is_null() { Body::empty() } else { Body::from(serde_json::to_vec(&call.body).unwrap_or_default()) };
        let mut req = Request::builder().method(call.method.as_str()).uri(&call.path).header("authorization", format!("Bearer {}", self.app.cfg().radio_token)).header(VIA_HEADER, via).header(airband_core::radio::SCHEMA_HEADER, &call.schema).extension(super::super::http::RadioId(self.keys.channel.clone())).header("content-type", "application/json");
        if now {
            req = req.header(PRESENCE_HEADER, serde_json::to_string(&call.presence).unwrap_or_default());
        }
        let Ok(req) = req.body(body) else {
            return self.refused(call, via, Refusal::BadAsk { reason: "that request cannot be made into one this computer can run".into() });
        };
        let Ok(res) = self.router.clone().oneshot(req).await;
        let status = res.status().as_u16();
        // the whole answer: one too big for the relay is told so when it is sealed for the relay
        // (`seal_answer_for_relay`), and one given straight has no such bound
        let Ok(bytes) = axum::body::to_bytes(res.into_body(), usize::MAX).await else {
            return self.refused(call, via, Refusal::NotNow { reason: "airband could not read its own answer.".into() });
        };
        match serde_json::from_slice(&bytes) {
            Ok(body) => (status, body),
            Err(_) if bytes.is_empty() => (status, Value::Null),
            // its own fault, and asking again gets the same: not something to wait out
            Err(e) => self.refused(call, via, Refusal::told(format!("airband made an answer that is not JSON ({e})."))),
        }
    }
}
