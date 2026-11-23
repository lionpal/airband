//! What the relay does for each route: a tower opening, pulling from, answering on and closing its channel, and
//! a radio calling through it, leaving a message on it, and a tower waking its phone.
use super::*;

/// A channel opened with a pull token that is not the one on record.
fn another_tower() -> Response {
    say(StatusCode::FORBIDDEN, "that channel belongs to another tower")
}

pub(super) async fn open(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let (true, false, Some(radio)) = (valid_channel(&channel), bearer(&headers).is_empty(), radio_hash_given(&headers)) else {
        return say(StatusCode::BAD_REQUEST, "a channel is 32 hex characters and needs a pull token and its radio's token hash");
    };
    let hash = token_hash(bearer(&headers));
    // opening again is how a tower checks it is still known after a relay restart, and what it does when it
    // starts over: nothing handed to it before is in its hands then
    if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
        if !same(&ch.pull_hash, &hash) {
            return another_tower();
        }
        ch.reopened();
        return StatusCode::NO_CONTENT.into_response();
    }
    if let Some(want) = &relay.admission {
        let given = header(&headers, ADMISSION_HEADER).unwrap_or("");
        if !same(given, want) {
            return say(StatusCode::UNAUTHORIZED, "this relay needs an admission key to open a channel");
        }
    }
    // a new channel, which anyone may ask for: so many from one address, and so many in all
    {
        let (mut limits, now) = (relay.limits.lock().unwrap(), Instant::now());
        if !limits.opened_by_address.allow(&caller(&headers), now) {
            return say(StatusCode::TOO_MANY_REQUESTS, "too many new pairings from this address today; try again tomorrow");
        }
        if !limits.opened.allow(&(), now) {
            return say(StatusCode::TOO_MANY_REQUESTS, "the relay is taking no new pairings for a while; try again in an hour");
        }
    }
    // durable first: a channel the relay would forget on restart is worse than a slow answer. What is on record
    // decides whose it is, so two towers opening one name at once, here or on two relays, are told the same.
    let on_record = match relay.store.add(&channel, &hash, radio).await {
        Err(e) => {
            eprintln!("airband-relay: could not save a new channel: {}", state::masked(&e, &channel));
            return say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not save the channel; try again");
        }
        // its tower closed it when the phone was unpaired: only a new pairing (a new channel) opens again. It
        // opened nothing, so it does not count against the address's new pairings: a tower that keeps asking
        // must not lock its own address out of pairing again
        Ok(store::Added::Closed) => {
            let mut limits = relay.limits.lock().unwrap();
            limits.opened_by_address.give_back(&caller(&headers));
            limits.opened.give_back(&());
            return say(StatusCode::GONE, "that pairing was ended; pair again");
        }
        Ok(store::Added::Open(saved)) => saved,
    };
    if !same(&on_record.pull_hash, &hash) {
        return another_tower();
    }
    relay.channels.lock().unwrap().entry(channel).or_insert_with(|| Channel::from(on_record));
    StatusCode::CREATED.into_response()
}

pub(super) async fn pull(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let until = Instant::now() + HOLD;
    let mut mine = None;
    loop {
        let wake = {
            let mut channels = relay.channels.lock().unwrap();
            let ch = match relay.as_tower(&mut channels, &channel, &headers) {
                Ok(ch) => ch,
                Err(r) => return *r,
            };
            // a tower that never says what it got would have everything handed to it again
            if mine.is_none() && header(&headers, PULL_HEADER) != Some(PULL_VERSION) {
                return say(StatusCode::UPGRADE_REQUIRED, "this relay needs a tower that says what each pull got; update airband");
            }
            let me = *mine.get_or_insert_with(|| ch.pulled(header(&headers, GOT_HEADER)));
            // a newer pull came: this one's connection is gone, and what it was handed would be lost with it
            if ch.pulls != me {
                return StatusCode::NO_CONTENT.into_response();
            }
            ch.last_pull = Some(Instant::now());
            // a radio that gave up leaves its request behind; skip those
            while let Some((id, sealed)) = ch.queue.pop_front() {
                if ch.waiting.contains_key(&id) {
                    ch.handed.push((id.clone(), sealed.clone()));
                    return ([(REQUEST_HEADER, id)], sealed).into_response();
                }
            }
            // then what was kept while it was away: one at a time, the oldest first, the next only once this one is taken
            (ch.wake.clone(), ch.kept_due())
        };
        let (wake, due) = wake;
        if let Some(seen) = due {
            match relay.store.held_next(&channel).await {
                Ok(Some((id, sealed))) => {
                    if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
                        ch.holding = Some((id.to_string(), state::Got::Handed));
                    }
                    return ([(REQUEST_HEADER, id.to_string()), (airband_core::relay::KEPT_HEADER, "1".to_string())], sealed).into_response();
                }
                Ok(None) => {
                    if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
                        ch.found_nothing_kept(seen);
                    }
                }
                Err(e) => eprintln!("airband-relay: cannot read what is kept for a channel: {}", state::masked(&e, &channel)),
            }
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return StatusCode::NO_CONTENT.into_response();
        }
        // wake at least every few seconds so last_pull stays current while the tower waits
        tokio::time::timeout(left.min(Duration::from_secs(5)), wake.notified()).await.ok();
    }
}

/// A tower closes its channel: the radio it was paired with has been cut off. Without this every channel ever
/// opened stayed on record.
pub(super) async fn close(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    if let Err(no) = relay.with_tower(&channel, &headers, |_| ()) {
        return *no;
    }
    // closed on record first: a failure leaves the channel open and the tower is told so (503), rather than a
    // channel gone here and back after a restart. Nothing here tries again: `airband unpair` says it was not
    // closed, and the channel stays on record, though its tower no longer answers on it. Once closed, a tower still running with that pairing that
    // opens it again is refused (410): nothing reopens it but a new pairing.
    if let Err(e) = relay.store.remove(&channel).await {
        eprintln!("airband-relay: cannot forget a channel: {}", state::masked(&e, &channel));
        return say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not forget the channel; try again");
    }
    relay.channels.lock().unwrap().remove(&channel);
    StatusCode::NO_CONTENT.into_response()
}

pub(super) async fn reply(State(relay): State<Arc<Relay>>, Path((channel, id)): Path<(String, String)>, headers: HeaderMap, sealed: Bytes) -> Response {
    // the tower took a message that was kept for it: there is no radio waiting on the answer, only the next one to send
    // said so by the tower in a header of its own, never read from how the id is written
    if headers.contains_key(airband_core::relay::KEPT_HEADER) {
        let kept = id.as_str();
        if let Err(no) = relay.with_tower(&channel, &headers, |_| ()) {
            return *no;
        }
        // gone from the store first: only then is the tower woken for the next one, so it is never handed this
        // one again, and a failure leaves it exactly as it was (still held, handed again in its time)
        // why stays in the relay's log: a store's words can name where it keeps things
        if let Err(e) = relay.store.held_done(&channel, kept).await {
            eprintln!("airband-relay: cannot let a kept message go: {}", state::masked(&e, &channel));
            return say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not let that message go; try again");
        }
        if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
            if ch.holding.as_ref().is_some_and(|(held, _)| held == kept) {
                ch.holding = None;
            }
            ch.wake.notify_one();
        }
        return StatusCode::NO_CONTENT.into_response();
    }
    match relay.with_tower(&channel, &headers, |ch| ch.waiting.remove(&id)) {
        Err(no) => *no,
        // the radio hung up between: told as for one that stopped waiting, never as delivered
        Ok(Some(radio)) => match radio.send(sealed) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(_) => say(StatusCode::GONE, "the radio stopped waiting for that answer"),
        },
        Ok(None) => say(StatusCode::GONE, "the radio stopped waiting for that answer"),
    }
}

/// Keep a sealed message for a tower that is away. The radio names it (`x-airband-id`), so sending it twice keeps
/// it once; the tower is handed what is kept in the order it came, as soon as it is back.
pub(super) async fn hold(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, sealed: Bytes) -> Response {
    let (began, bytes) = (Instant::now(), sealed.len());
    let answer = keep(&relay, &channel, &headers, sealed).await;
    relay.note_failed("queue", &channel, answer.status(), began, bytes, StatusCode::ACCEPTED);
    answer
}

pub(super) async fn keep(relay: &Arc<Relay>, channel: &str, headers: &HeaderMap, sealed: Bytes) -> Response {
    let id = header(headers, ID_HEADER).unwrap_or("");
    if !airband_core::encode::is_hex(id, 8..=40) || sealed.is_empty() {
        return say(StatusCode::BAD_REQUEST, "a kept message needs an id and a body");
    }
    if let Err(no) = relay.admit(Asks::Queue, channel, headers) {
        return *no;
    }
    match relay.store.hold(channel, id, &sealed, HELD_MOST).await {
        Ok(store::Held::Kept) => {
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(channel) {
                ch.kept_one();
            }
            StatusCode::ACCEPTED.into_response()
        }
        Ok(store::Held::Full) => say(StatusCode::INSUFFICIENT_STORAGE, "too many messages are already waiting for that tower"),
        Ok(store::Held::Closed) => say(StatusCode::GONE, "that pairing was ended"),
        Err(e) => {
            eprintln!("airband-relay: cannot keep a message: {}", state::masked(&e, channel));
            say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not keep it")
        }
    }
}

pub(super) async fn call(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, sealed: Bytes) -> Response {
    let (began, bytes) = (Instant::now(), sealed.len());
    let answer = carry(&relay, &channel, &headers, sealed).await;
    relay.note_failed("call", &channel, answer.status(), began, bytes, StatusCode::OK);
    answer
}

pub(super) async fn carry(relay: &Arc<Relay>, channel: &str, headers: &HeaderMap, sealed: Bytes) -> Response {
    if let Err(no) = relay.admit(Asks::Call, channel, headers) {
        return *no;
    }
    let id = random_hex(12);
    let answer = {
        let mut channels = relay.channels.lock().unwrap();
        let Some(ch) = channels.get_mut(channel) else { return not_the_radio() };
        if ch.last_pull.is_none_or(|t| t.elapsed() > AIRBAND_GONE) {
            return say(StatusCode::SERVICE_UNAVAILABLE, "the tower is not connected to the relay");
        }
        // what he says must arrive in the order he said it: while older words are still being handed over, new
        // ones are turned back to join the end of that line
        if ch.has_held && headers.contains_key(ORDERED_HEADER) {
            return say(StatusCode::CONFLICT, "earlier messages are still on their way to the tower");
        }
        if ch.waiting.len() >= MAX_IN_FLIGHT {
            return say(StatusCode::TOO_MANY_REQUESTS, "too many requests in flight on this channel");
        }
        let (tx, rx) = oneshot::channel();
        ch.waiting.insert(id.clone(), tx);
        ch.queue.push_back((id.clone(), sealed));
        ch.wake.notify_one();
        rx
    };
    // nothing of a request nobody waits for stays in memory: when the wait ends, or when the radio hangs up and
    // this is dropped, it goes, so one the tower never answers cannot fill the channel's share
    let _waiting = Waiting { relay: relay.clone(), channel: channel.to_string(), id };
    match tokio::time::timeout(ANSWER_WAIT, answer).await {
        Ok(Ok(sealed)) => sealed.into_response(),
        _ => say(StatusCode::GATEWAY_TIMEOUT, "the tower did not answer"),
    }
}

/// A radio's request while its radio waits. Dropped, the request goes from the channel, answered or not.
struct Waiting {
    relay: Arc<Relay>,
    channel: String,
    id: String,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if let Some(ch) = self.relay.channels.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&self.channel) {
            ch.waiting.remove(&self.id);
            ch.queue.retain(|(queued, _)| queued != &self.id);
            ch.handed.retain(|(handed, _)| handed != &self.id);
        }
    }
}

/// A tower asks for its phone to be woken: a sealed notification, handed to the push service the phone's
/// address belongs to. Only the channel's own tower may ask, each channel only so often, and nothing of what
/// is passed on can be read here. Neither the address nor the notification is logged or kept.
pub(super) async fn wake_phone(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let not = |code: StatusCode, why: NotPushed| (code, axum::Json(PushAnswer::not_sent(why))).into_response();
    if !relay.limits.lock().unwrap().push_by_address.allow(&caller(&headers), Instant::now()) {
        return not(StatusCode::TOO_MANY_REQUESTS, NotPushed::TooManyFromAddress);
    }
    match relay.with_tower(&channel, &headers, |ch| push::allow(&mut ch.pushed, Instant::now())) {
        Err(no) => return *no,
        Ok(false) => return not(StatusCode::TOO_MANY_REQUESTS, NotPushed::TooManyOnChannel),
        Ok(true) => {}
    }
    let Ok(ask) = serde_json::from_slice::<PushRequest>(&body) else { return not(StatusCode::BAD_REQUEST, NotPushed::NotARequest) };
    if !ask.well_formed() {
        return not(StatusCode::BAD_REQUEST, NotPushed::WrongShape);
    }
    match relay.push.send(ask.provider, &ask.to, &ask.sealed, ask.question).await {
        None => not(StatusCode::SERVICE_UNAVAILABLE, NotPushed::NoSuchService),
        Some(push::Outcome::Sent) => (StatusCode::OK, axum::Json(PushAnswer { sent: true, ..PushAnswer::default() })).into_response(),
        Some(push::Outcome::Gone) => not(StatusCode::GONE, NotPushed::Gone),
        Some(push::Outcome::Refused(said)) => {
            // the service's own words are for this log only; the tower is told the status, as a fact
            eprintln!("airband-relay: a notification was refused: {said}");
            not(StatusCode::BAD_GATEWAY, NotPushed::Refused { status: said.status })
        }
        Some(push::Outcome::Failed(why)) => {
            eprintln!("airband-relay: a notification could not be sent: {why}");
            not(StatusCode::BAD_GATEWAY, why.told())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::relay::Keys;

    /// A relay on a folder of its own, with nothing configured, and that folder.
    fn relay_in_a_folder(admission: Option<&str>) -> (Arc<Relay>, std::path::PathBuf) {
        let folder = std::env::temp_dir().join(format!("airband-relay-test-{}", random_hex(6)));
        std::fs::create_dir_all(&folder).unwrap();
        (Arc::new(Relay::new(push::Push::default(), HashMap::new(), store::folder(folder.clone()), admission.map(str::to_string))), folder)
    }
    fn with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), value.parse().unwrap());
        }
        headers
    }
    fn token(t: &str) -> (&'static str, String) {
        ("authorization", format!("Bearer {t}"))
    }
    /// A pull's answer: its status, the id it hands over and what is sealed in it.
    async fn text_with_id(res: impl std::future::Future<Output = Response>) -> (u16, Option<String>, String) {
        let res = res.await;
        let id = res.headers().get(REQUEST_HEADER).map(|v| v.to_str().unwrap().to_string());
        let (status, body) = text(res).await;
        (status, id, body)
    }
    async fn text(res: Response) -> (u16, String) {
        let status = res.status().as_u16();
        (status, String::from_utf8_lossy(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).to_string())
    }
    /// A tower opens its channel, saying who its radio is.
    async fn open_as(relay: &Arc<Relay>, keys: &Keys, admission: &str) -> u16 {
        let (auth, hash) = (token(&keys.pull_token), token_hash(&keys.radio_token));
        open(State(relay.clone()), Path(keys.channel.clone()), with(&[(auth.0, &auth.1), (RADIO_HASH_HEADER, &hash), (ADMISSION_HEADER, admission)])).await.status().as_u16()
    }
    /// A radio leaves a message, showing `shown` as its token (none when empty).
    async fn leave(relay: &Arc<Relay>, channel: &str, shown: &str, id: &str) -> (u16, String) {
        let auth = token(shown);
        let mut headers = with(&[(ID_HEADER, id)]);
        if !shown.is_empty() {
            headers.insert(auth.0, auth.1.parse().unwrap());
        }
        text(hold(State(relay.clone()), Path(channel.to_string()), headers, Bytes::from_static(b"sealed")).await).await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_kept_message_and_a_channel_leave_memory_only_once_the_store_has_let_them_go() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        assert_eq!(leave(&relay, &keys.channel, &keys.radio_token, "aaaaaaaa01").await.0, 202);
        let tower = pulling(&keys, None);
        let handed = pull(State(relay.clone()), Path(keys.channel.clone()), tower.clone()).await;
        assert_eq!(handed.headers().get(REQUEST_HEADER).unwrap(), "aaaaaaaa01");
        // the store cannot let it go: the answer says so, and it is still held, as it was
        // (its folder cannot be written just now)
        use std::os::unix::fs::PermissionsExt;
        let held = folder.join("held").join(&keys.channel);
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o500)).unwrap();
        let mut kept = tower.clone();
        kept.insert(airband_core::relay::KEPT_HEADER, "1".parse().unwrap());
        let said = reply(State(relay.clone()), Path((keys.channel.clone(), "aaaaaaaa01".into())), kept.clone(), Bytes::new()).await;
        assert_eq!(said.status().as_u16(), 503);
        assert!(relay.channels.lock().unwrap()[&keys.channel].holding.is_some());
        // once it can, it is let go, and only then is the holding cleared
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(reply(State(relay.clone()), Path((keys.channel.clone(), "aaaaaaaa01".into())), kept, Bytes::new()).await.status().as_u16(), 204);
        assert!(relay.channels.lock().unwrap()[&keys.channel].holding.is_none());
        assert_eq!(relay.store.held_next(&keys.channel).await, Ok(None));
        // closing: a store that cannot forget it leaves the channel known, so trying again finds it
        let channels = folder.join("channels.json");
        let good = std::fs::read(&channels).unwrap();
        std::fs::write(&channels, b"{broken").unwrap();
        assert_eq!(close(State(relay.clone()), Path(keys.channel.clone()), tower.clone()).await.status().as_u16(), 503);
        assert!(relay.channels.lock().unwrap().contains_key(&keys.channel));
        std::fs::write(&channels, good).unwrap();
        assert_eq!(close(State(relay.clone()), Path(keys.channel.clone()), tower.clone()).await.status().as_u16(), 204);
        assert!(!relay.channels.lock().unwrap().contains_key(&keys.channel));
        // a tower still running with that pairing finds it gone and opens it again: refused, and nothing kept
        assert_eq!(pull(State(relay.clone()), Path(keys.channel.clone()), tower).await.status().as_u16(), 404);
        assert_eq!(open_as(&relay, &keys, "").await, 410);
        assert!(!relay.channels.lock().unwrap().contains_key(&keys.channel));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn opening_a_closed_channel_again_and_again_never_uses_up_the_addresss_new_pairings() {
        // M1: each refused open counted against the ten a day, so the tower's own address could pair no more
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        assert_eq!(close(State(relay.clone()), Path(keys.channel.clone()), pulling(&keys, None)).await.status().as_u16(), 204);
        for _ in 0..30 {
            assert_eq!(open_as(&relay, &keys, "").await, 410);
        }
        let fresh = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &fresh, "").await, 201);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn two_towers_opening_one_channel_are_told_whose_it_is_by_the_record() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        // the other tower opened it first, on another relay sharing the store: this one's memory knows nothing
        relay.store.add(&keys.channel, &token_hash("another tower"), "r").await.unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 403);
        assert!(!relay.channels.lock().unwrap().contains_key(&keys.channel));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// What a tower's pull carries: its token, the pull it speaks, and what its previous pull got.
    fn pulling(keys: &Keys, got: Option<&str>) -> HeaderMap {
        let auth = token(&keys.pull_token);
        let mut headers = with(&[(auth.0, &auth.1), (PULL_HEADER, PULL_VERSION)]);
        if let Some(got) = got {
            headers.insert(GOT_HEADER, got.parse().unwrap());
        }
        headers
    }
    /// A radio's request waiting for its tower, as `carry` leaves it: its radio waits as long as the receiver lives.
    fn waiting_on(relay: &Arc<Relay>, keys: &Keys, id: &str) -> tokio::sync::oneshot::Receiver<Bytes> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut channels = relay.channels.lock().unwrap();
        let ch = channels.get_mut(&keys.channel).unwrap();
        ch.waiting.insert(id.into(), tx);
        ch.queue.push_back((id.into(), Bytes::from_static(b"sealed")));
        rx
    }

    #[tokio::test]
    async fn a_request_handed_to_a_pull_whose_connection_died_is_handed_to_the_next() {
        // M3: written out in full to a connection that had silently gone, it was lost while its radio waited
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        let _radio = waiting_on(&relay, &keys, "r1");
        let pull_now = |got: Option<&str>| text_with_id(pull(State(relay.clone()), Path(keys.channel.clone()), pulling(&keys, got)));
        assert_eq!(pull_now(None).await, (200, Some("r1".into()), "sealed".into()));
        // the next pull does not say it got it: it never arrived, and it is handed over again
        assert_eq!(pull_now(None).await, (200, Some("r1".into()), "sealed".into()));
        // one that says it got it is not handed it again
        let _other = waiting_on(&relay, &keys, "r2");
        assert_eq!(pull_now(Some("r1")).await, (200, Some("r2".into()), "sealed".into()));
        {
            let channels = relay.channels.lock().unwrap();
            assert!(channels[&keys.channel].queue.is_empty());
            assert_eq!(channels[&keys.channel].handed.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["r2"]);
        }
        // one whose radio stopped waiting is not handed again
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().waiting.remove("r2");
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().pulled(None);
        assert!(relay.channels.lock().unwrap()[&keys.channel].queue.is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn an_older_pull_still_held_open_takes_nothing_once_a_newer_one_comes() {
        // a dead pull is still waiting on the channel; a request must go to the live one
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        let old = tokio::spawn(pull(State(relay.clone()), Path(keys.channel.clone()), pulling(&keys, None)));
        while relay.channels.lock().unwrap()[&keys.channel].pulls == 0 {
            tokio::task::yield_now().await;
        }
        let new = tokio::spawn(text_with_id(pull(State(relay.clone()), Path(keys.channel.clone()), pulling(&keys, None))));
        while relay.channels.lock().unwrap()[&keys.channel].pulls == 1 {
            tokio::task::yield_now().await;
        }
        assert_eq!(old.await.unwrap().status().as_u16(), 204);
        let _radio = waiting_on(&relay, &keys, "r1");
        relay.channels.lock().unwrap()[&keys.channel].wake.notify_waiters();
        assert_eq!(new.await.unwrap(), (200, Some("r1".into()), "sealed".into()));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn a_kept_message_no_pull_said_it_got_is_handed_again_and_one_got_waits_for_its_answer() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        assert_eq!(leave(&relay, &keys.channel, &keys.radio_token, "aaaaaaaa01").await.0, 202);
        let pull_now = |got: Option<&str>| text_with_id(pull(State(relay.clone()), Path(keys.channel.clone()), pulling(&keys, got)));
        assert_eq!(pull_now(None).await.1.as_deref(), Some("aaaaaaaa01"));
        // never got: handed again at once, with no clock deciding it
        assert_eq!(pull_now(None).await.1.as_deref(), Some("aaaaaaaa01"));
        // got: the tower is answering for it, and the next pull waits rather than be handed it twice
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().pulled(Some("aaaaaaaa01"));
        assert_eq!(relay.channels.lock().unwrap()[&keys.channel].holding, Some(("aaaaaaaa01".into(), state::Got::Got)));
        assert_eq!(relay.channels.lock().unwrap()[&keys.channel].kept_due(), None);
        // a tower that starts over opens its channel again: then it is handed again
        assert_eq!(open_as(&relay, &keys, "").await, 204);
        assert!(relay.channels.lock().unwrap()[&keys.channel].kept_due().is_some());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn a_radio_that_hangs_up_leaves_nothing_behind_and_its_answer_is_not_taken_as_delivered() {
        // R-N3: only carry's own timeout cleaned up, so requests the tower never answered filled the channel
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().last_pull = Some(Instant::now());
        let auth = token(&keys.radio_token);
        let radio = with(&[(auth.0, &auth.1)]);
        let calling = tokio::spawn(call(State(relay.clone()), Path(keys.channel.clone()), radio, Bytes::from_static(b"sealed")));
        while relay.channels.lock().unwrap()[&keys.channel].waiting.is_empty() {
            tokio::task::yield_now().await;
        }
        let id = relay.channels.lock().unwrap()[&keys.channel].queue[0].0.clone();
        // handed over, then the radio hangs up
        assert_eq!(text_with_id(pull(State(relay.clone()), Path(keys.channel.clone()), pulling(&keys, None))).await.1, Some(id.clone()));
        calling.abort();
        assert!(calling.await.is_err());
        {
            let channels = relay.channels.lock().unwrap();
            let ch = &channels[&keys.channel];
            assert!(ch.waiting.is_empty() && ch.queue.is_empty() && ch.handed.is_empty());
        }
        // the tower's answer finds nobody: told so, never 204
        let tower = pulling(&keys, None);
        assert_eq!(reply(State(relay.clone()), Path((keys.channel.clone(), id)), tower, Bytes::from_static(b"answer")).await.status().as_u16(), 410);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn a_pull_that_does_not_say_what_it_got_is_refused() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        let _radio = waiting_on(&relay, &keys, "r1");
        let auth = token(&keys.pull_token);
        assert_eq!(pull(State(relay.clone()), Path(keys.channel.clone()), with(&[(auth.0, &auth.1)])).await.status().as_u16(), 426);
        assert_eq!(relay.channels.lock().unwrap()[&keys.channel].queue.len(), 1, "nothing was handed over");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn nobody_sends_to_a_channel_that_is_not_theirs() {
        let (relay, folder) = relay_in_a_folder(None);
        let (mine, other) = (Keys::derive(&airband_core::relay::new_secret()).unwrap(), Keys::derive(&airband_core::relay::new_secret()).unwrap());
        assert_eq!(open_as(&relay, &mine, "").await, 201);
        assert_eq!(open_as(&relay, &other, "").await, 201);
        // the radio's token hash is kept with the channel from the moment it is opened
        assert_eq!(relay.store.load().await.unwrap()[&mine.channel].radio_hash, token_hash(&mine.radio_token));

        // nothing without the token, with a wrong one, or with another channel's
        let refused = leave(&relay, &mine.channel, "", "aaaaaaaa05").await;
        assert_eq!(refused.0, 401);
        assert_eq!(leave(&relay, &mine.channel, "not-the-token", "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &mine.channel, &other.radio_token, "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &mine.channel, &mine.radio_token, "aaaaaaaa05").await.0, 202);
        // a channel that does not exist answers exactly as one that refuses
        assert_eq!(leave(&relay, &"0".repeat(32), "", "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &"0".repeat(32), &mine.radio_token, "aaaaaaaa05").await, refused);
        // the other channel takes its own radio's token and not this one's
        assert_eq!(leave(&relay, &other.channel, &mine.radio_token, "aaaaaaaa06").await, refused);
        assert_eq!(leave(&relay, &other.channel, &other.radio_token, "aaaaaaaa06").await.0, 202);

        // a call is under the same rule: refused without the token, let through to "no tower" with it
        let ask = |headers: HeaderMap| call(State(relay.clone()), Path(mine.channel.clone()), headers, Bytes::from_static(b"sealed"));
        let auth = token(&mine.radio_token);
        assert_eq!(text(ask(HeaderMap::new()).await).await, refused);
        assert_eq!(text(ask(with(&[("authorization", "Bearer nope")])).await).await, refused);
        assert_eq!(ask(with(&[(auth.0, &auth.1)])).await.status().as_u16(), 503);
        assert_eq!(call(State(relay.clone()), Path("0".repeat(32)), HeaderMap::new(), Bytes::from_static(b"sealed")).await.status().as_u16(), 401);

        // a relay started again on the same folder remembers all of it
        let saved = store::folder(folder.clone()).load().await.unwrap();
        let again = Arc::new(Relay::new(push::Push::default(), saved.into_iter().map(|(c, s)| (c, Channel::from(s))).collect(), store::folder(folder.clone()), None));
        assert_eq!(leave(&again, &mine.channel, "", "aaaaaaaa07").await, refused);
        assert_eq!(leave(&again, &mine.channel, &mine.radio_token, "aaaaaaaa07").await.0, 202);
        assert_eq!(leave(&again, &other.channel, &other.radio_token, "aaaaaaaa08").await.0, 202);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn an_open_relay_takes_so_many_new_channels_from_one_address() {
        let (relay, folder) = relay_in_a_folder(None);
        let new = || Keys::derive(&airband_core::relay::new_secret()).unwrap();
        let first = new();
        assert_eq!(open_as(&relay, &first, "").await, 201);
        for _ in 1..OPENED_AN_ADDRESS_A_DAY {
            assert_eq!(open_as(&relay, &new(), "").await, 201);
        }
        assert_eq!(open_as(&relay, &new(), "").await, 429);
        // a channel it already has is not a new one: its tower can still look in
        assert_eq!(open_as(&relay, &first, "").await, 204);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn a_tower_needs_its_channels_pull_token_for_everything_and_the_admission_key_to_open_one() {
        let (relay, folder) = relay_in_a_folder(Some("key"));
        let (mine, other) = (Keys::derive(&airband_core::relay::new_secret()).unwrap(), Keys::derive(&airband_core::relay::new_secret()).unwrap());
        // a new channel: not without the admission key
        assert_eq!(open_as(&relay, &mine, "wrong").await, 401);
        assert_eq!(open_as(&relay, &mine, "key").await, 201);
        // opening it again needs no key, only the same token; another tower's token is turned away
        assert_eq!(open_as(&relay, &mine, "").await, 204);
        let theirs = token(&other.pull_token);
        let their_radio = token_hash(&other.radio_token);
        let as_other = || with(&[(theirs.0, &theirs.1), (ADMISSION_HEADER, "key"), (RADIO_HASH_HEADER, &their_radio)]);
        let channel = || Path(mine.channel.clone());
        assert_eq!(open(State(relay.clone()), channel(), as_other()).await.status().as_u16(), 403);
        // and so is everything else a tower does, with another channel's token or with none
        for headers in [as_other(), HeaderMap::new()] {
            assert_eq!(pull(State(relay.clone()), channel(), headers.clone()).await.status().as_u16(), 403);
            assert_eq!(reply(State(relay.clone()), Path((mine.channel.clone(), "abc".into())), headers.clone(), Bytes::new()).await.status().as_u16(), 403);
            assert_eq!(reply(State(relay.clone()), Path((mine.channel.clone(), "held-abc".into())), headers.clone(), Bytes::new()).await.status().as_u16(), 403);
            assert_eq!(wake_phone(State(relay.clone()), channel(), headers.clone(), Bytes::new()).await.status().as_u16(), 403);
            assert_eq!(close(State(relay.clone()), channel(), headers).await.status().as_u16(), 403);
        }
        // who this channel's radio is was said when it was opened, and nobody changes it
        assert_eq!(relay.channels.lock().unwrap()[&mine.channel].radio_hash, token_hash(&mine.radio_token));
        // a channel is not opened without saying who its radio is
        let fresh = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        let own = token(&fresh.pull_token);
        assert_eq!(open(State(relay.clone()), Path(fresh.channel.clone()), with(&[(own.0, &own.1), (ADMISSION_HEADER, "key")])).await.status().as_u16(), 400);
        // its own tower closes it, and it is gone
        let own = token(&mine.pull_token);
        assert_eq!(close(State(relay.clone()), channel(), with(&[(own.0, &own.1)])).await.status().as_u16(), 204);
        assert!(relay.store.load().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn only_so_much_is_taken_from_one_channel_and_from_one_address() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        let auth = token(&keys.radio_token);
        let from = |address: &str| with(&[(auth.0, &auth.1), (ID_HEADER, "dddddddd01"), ("x-forwarded-for", address)]);
        let leave = |address: &'static str| hold(State(relay.clone()), Path(keys.channel.clone()), from(address), Bytes::from_static(b"sealed"));
        // the channel's share for a minute, from two addresses, and then no more from anywhere
        for i in 0..KEPT_A_MINUTE {
            assert_eq!(leave(if i % 2 == 0 { "198.51.100.1" } else { "198.51.100.2" }).await.status().as_u16(), 202, "{i}");
        }
        assert_eq!(leave("198.51.100.3").await.status().as_u16(), 429);
        // one address asking about channels it has no token for uses up its own share and nobody else's
        let stranger = || hold(State(relay.clone()), Path("0".repeat(32)), with(&[(ID_HEADER, "dddddddd01"), ("x-forwarded-for", "203.0.113.9")]), Bytes::from_static(b"sealed"));
        for _ in 0..KEPT_A_MINUTE * 2 {
            assert_eq!(stranger().await.status().as_u16(), 401);
        }
        assert_eq!(stranger().await.status().as_u16(), 429);
        let _ = std::fs::remove_dir_all(folder);
    }

    /// The push route, with a stand-in for Apple's server on this machine: the tower's token is asked for, the
    /// sealed notification reaches the service untouched under a pass signed by the key, and what the service
    /// says about an address comes back as the tower needs it.
    #[tokio::test]
    async fn a_tower_wakes_its_phone_through_the_push_service() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let (key, public) = push::tests::test_key();
        let seen = Arc::new(AtomicU32::new(0));
        // the stand-in: checks what a push service checks, and answers by the address
        let apple = {
            let seen = seen.clone();
            Router::new().route(
                "/3/device/{to}",
                post(move |Path(to): Path<String>, headers: HeaderMap, body: Bytes| {
                    let (public, seen) = (public.clone(), seen.clone());
                    async move {
                        let head = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                        let pass = head("authorization");
                        let sent: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                        let good = push::tests::pass_is_good(pass.strip_prefix("bearer ").unwrap_or(""), &public, "KEY123", "TEAM456") && head("apns-topic") == "dev.example.app" && head("apns-push-type") == "alert" && sent["aps"]["mutable-content"] == 1 && sent["sealed"] == "c2VhbGVk" && sent["aps"]["alert"]["body"] == "A question for you";
                        if !good {
                            return (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "reason": "InvalidProviderToken" })));
                        }
                        seen.fetch_add(1, Ordering::SeqCst);
                        match to.as_str() {
                            "aa11" => (StatusCode::OK, axum::Json(serde_json::json!({}))),
                            "bb22" => (StatusCode::GONE, axum::Json(serde_json::json!({ "reason": "Unregistered" }))),
                            "dd44" => (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "reason": "ServiceUnavailable" }))),
                            _ => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "reason": "BadDeviceToken" }))),
                        }
                    }
                }),
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, apple).await });

        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        let apns = push::Apns::new(&key, "KEY123", "TEAM456", "dev.example.app", &format!("http://{at}")).unwrap();
        let folder = std::env::temp_dir().join(format!("airband-relay-test-{}", random_hex(6)));
        let relay = Arc::new(Relay::new(push::Push::with_apns(apns), HashMap::from([(keys.channel.clone(), Channel::new(token_hash(&keys.pull_token), token_hash(&keys.radio_token)))]), store::folder(folder), None));
        let ask = |token: &str, provider: airband_core::relay::PushService, to: &str| {
            let (relay, channel) = (relay.clone(), keys.channel.clone());
            let mut headers = HeaderMap::new();
            headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
            let body = serde_json::to_vec(&PushRequest { provider, to: to.into(), sealed: "c2VhbGVk".into(), question: true }).unwrap();
            async move {
                let res = wake_phone(State(relay), Path(channel), headers, Bytes::from(body)).await;
                let status = res.status().as_u16();
                let answer: PushAnswer = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap_or_default();
                (status, answer.sent, answer.gone, answer.why)
            }
        };
        // not the channel's tower: nothing is sent
        assert_eq!(ask("someone-else", airband_core::relay::PushService::Apns, "aa11").await.0, 403);
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11").await, (200, true, false, None));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "bb22").await, (410, false, true, Some(NotPushed::Gone)));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "cc33").await, (410, false, true, Some(NotPushed::Gone)));
        // the service's own word for a refusal is never passed on: the tower is told its status
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "dd44").await, (502, false, false, Some(NotPushed::Refused { status: 503 })));
        assert_eq!(seen.load(Ordering::SeqCst), 4);
        // a service this relay does not have, and an address that is not one
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Unknown, "aa11").await.0, 503);
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11/../x").await.0, 400);
        // and only so many from one channel
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().pushed = Some((Instant::now(), push::PUSHES));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11").await.0, 429);
        assert_eq!(seen.load(Ordering::SeqCst), 4);
    }
}
