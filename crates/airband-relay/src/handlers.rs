//! What the relay does for each route: a tower opening, pulling from, answering on and closing its channel, and
//! a radio calling through it, leaving a message on it, and a tower waking its phone.
use super::*;

pub(super) async fn open(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let (true, false, Some(radio)) = (valid_channel(&channel), bearer(&headers).is_empty(), radio_hash_given(&headers)) else {
        return say(StatusCode::BAD_REQUEST, "a channel is 32 hex characters and needs a pull token and its radio's token hash");
    };
    let hash = token_hash(bearer(&headers));
    // opening again is how a tower checks it is still known after a relay restart
    let known = |channels: &HashMap<String, Channel>| channels.get(&channel).map(|existing| if same(&existing.pull_hash, &hash) { StatusCode::NO_CONTENT.into_response() } else { say(StatusCode::FORBIDDEN, "that channel belongs to another tower") });
    let answer = known(&relay.channels.lock().unwrap());
    if let Some(answer) = answer {
        return answer;
    }
    if let Some(want) = &relay.admission {
        let given = headers.get(ADMISSION_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
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
        if !limits.opened.allow("all", now) {
            return say(StatusCode::TOO_MANY_REQUESTS, "the relay is taking no new pairings for a while; try again in an hour");
        }
    }
    // durable first: a channel the relay would forget on restart is worse than a slow answer
    if let Err(e) = relay.store.add(&channel, &hash, radio).await {
        eprintln!("airband-relay: could not save a new channel: {e}");
        return say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not save the channel; try again");
    }
    let mut channels = relay.channels.lock().unwrap();
    // two towers raced for the same name: the first one in keeps it
    if let Some(answer) = known(&channels) {
        return answer;
    }
    channels.insert(channel, Channel::from(store::Saved { pull_hash: hash, radio_hash: radio.to_string() }));
    StatusCode::CREATED.into_response()
}

pub(super) async fn pull(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let until = Instant::now() + HOLD;
    loop {
        let wake = {
            let mut channels = relay.channels.lock().unwrap();
            let ch = match relay.as_tower(&mut channels, &channel, &headers) {
                Ok(ch) => ch,
                Err(r) => return *r,
            };
            ch.last_pull = Some(Instant::now());
            // a radio that gave up leaves its request behind; skip those
            while let Some((id, sealed)) = ch.queue.pop_front() {
                if ch.waiting.contains_key(&id) {
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
                        ch.holding = Some((id.clone(), Instant::now()));
                    }
                    return ([(REQUEST_HEADER, id.to_string()), (airband_core::relay::KEPT_HEADER, "1".to_string())], sealed).into_response();
                }
                Ok(None) => {
                    if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
                        ch.found_nothing_kept(seen);
                    }
                }
                Err(e) => eprintln!("airband-relay: cannot read what is kept for a channel: {e}"),
            }
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return StatusCode::NO_CONTENT.into_response();
        }
        // wake at least every few seconds so last_pull stays current while the tower waits
        let _ = tokio::time::timeout(left.min(Duration::from_secs(5)), wake.notified()).await;
    }
}

/// A tower closes its channel: the radio it was paired with has been cut off. Without this every channel ever
/// opened stayed on record.
pub(super) async fn close(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    if let Err(no) = relay.with_tower(&channel, &headers, |_| ()) {
        return *no;
    }
    // forgotten on record first: a failure leaves the channel as it was, so the tower's retry finds it again
    // instead of a 404 now and the channel coming back after a restart
    if let Err(e) = relay.store.remove(&channel).await {
        eprintln!("airband-relay: cannot forget a channel: {e}");
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
        if let Err(e) = relay.store.held_done(&channel, kept).await {
            return say(StatusCode::SERVICE_UNAVAILABLE, &e);
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
        Ok(Some(radio)) => {
            let _ = radio.send(sealed);
            StatusCode::NO_CONTENT.into_response()
        }
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
    let id = headers.get(ID_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !airband_core::encode::is_hex(id, 8..=40) || sealed.is_empty() {
        return say(StatusCode::BAD_REQUEST, "a kept message needs an id and a body");
    }
    if let Err(no) = relay.admit(Asks::Queue, channel, headers) {
        return *no;
    }
    match relay.store.hold(channel, id, &sealed, HELD_MOST).await {
        Ok(true) => {
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(channel) {
                ch.kept_one();
            }
            StatusCode::ACCEPTED.into_response()
        }
        Ok(false) => say(StatusCode::INSUFFICIENT_STORAGE, "too many messages are already waiting for that tower"),
        Err(e) => {
            eprintln!("airband-relay: cannot keep a message: {e}");
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
    match tokio::time::timeout(ANSWER_WAIT, answer).await {
        Ok(Ok(sealed)) => sealed.into_response(),
        _ => {
            // nothing of a request nobody waits for stays in memory
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(channel) {
                ch.waiting.remove(&id);
                ch.queue.retain(|(queued, _)| queued != &id);
            }
            say(StatusCode::GATEWAY_TIMEOUT, "the tower did not answer")
        }
    }
}

/// A tower asks for its phone to be woken: a sealed notification, handed to the push service the phone's
/// address belongs to. Only the channel's own tower may ask, each channel only so often, and nothing of what
/// is passed on can be read here. Neither the address nor the notification is logged or kept.
pub(super) async fn wake_phone(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let answer = |code: StatusCode, sent: bool, gone: bool, reason: &str| (code, axum::Json(PushAnswer { sent, gone, reason: reason.into() })).into_response();
    if !relay.limits.lock().unwrap().push_by_address.allow(&caller(&headers), Instant::now()) {
        return answer(StatusCode::TOO_MANY_REQUESTS, false, false, "too many notifications from this address; try again later");
    }
    match relay.with_tower(&channel, &headers, |ch| push::allow(&mut ch.pushed, Instant::now())) {
        Err(no) => return *no,
        Ok(false) => return answer(StatusCode::TOO_MANY_REQUESTS, false, false, "too many notifications from this channel; try again later"),
        Ok(true) => {}
    }
    let Ok(ask) = serde_json::from_slice::<PushRequest>(&body) else { return answer(StatusCode::BAD_REQUEST, false, false, "not a push request") };
    if !ask.well_formed() {
        return answer(StatusCode::BAD_REQUEST, false, false, "the address or the sealed notification is not the right shape");
    }
    match relay.push.send(ask.provider, &ask.to, &ask.sealed, ask.question).await {
        None => answer(StatusCode::SERVICE_UNAVAILABLE, false, false, "this relay has no such push service"),
        Some(push::Outcome::Sent) => answer(StatusCode::OK, true, false, ""),
        Some(push::Outcome::Gone(why)) => answer(StatusCode::GONE, false, true, &why),
        Some(push::Outcome::Refused(why)) => {
            eprintln!("airband-relay: the push service refused a notification: {why}");
            answer(StatusCode::BAD_GATEWAY, false, false, &why)
        }
        Some(push::Outcome::Failed(why)) => {
            eprintln!("airband-relay: a notification could not be sent: {why}");
            answer(StatusCode::BAD_GATEWAY, false, false, &why)
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
        (Arc::new(Relay::new(push::Push::default(), HashMap::new(), store::Store::Folder(folder.clone()), admission.map(str::to_string))), folder)
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

    #[tokio::test]
    async fn a_kept_message_and_a_channel_leave_memory_only_once_the_store_has_let_them_go() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        assert_eq!(leave(&relay, &keys.channel, &keys.radio_token, "aaaaaaaa01").await.0, 202);
        let auth = token(&keys.pull_token);
        let tower = with(&[(auth.0, &auth.1)]);
        let handed = pull(State(relay.clone()), Path(keys.channel.clone()), tower.clone()).await;
        assert_eq!(handed.headers().get(REQUEST_HEADER).unwrap(), "aaaaaaaa01");
        // the store cannot let it go: the answer says so, and it is still held, as it was
        let list = folder.join("held").join(&keys.channel).join("list.json");
        let good = std::fs::read(&list).unwrap();
        std::fs::write(&list, b"{broken").unwrap();
        let mut kept = tower.clone();
        kept.insert(airband_core::relay::KEPT_HEADER, "1".parse().unwrap());
        let said = reply(State(relay.clone()), Path((keys.channel.clone(), "aaaaaaaa01".into())), kept.clone(), Bytes::new()).await;
        assert_eq!(said.status().as_u16(), 503);
        assert!(relay.channels.lock().unwrap()[&keys.channel].holding.is_some());
        // once it can, it is let go, and only then is the holding cleared
        std::fs::write(&list, good).unwrap();
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
        assert_eq!(close(State(relay.clone()), Path(keys.channel.clone()), tower).await.status().as_u16(), 204);
        assert!(!relay.channels.lock().unwrap().contains_key(&keys.channel));
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
        let saved = store::Store::Folder(folder.clone()).load().await.unwrap();
        let again = Arc::new(Relay::new(push::Push::default(), saved.into_iter().map(|(c, s)| (c, Channel::from(s))).collect(), store::Store::Folder(folder.clone()), None));
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
        let relay = Arc::new(Relay::new(push::Push::with_apns(apns), HashMap::from([(keys.channel.clone(), Channel::new(token_hash(&keys.pull_token), token_hash(&keys.radio_token)))]), store::Store::Folder(folder), None));
        let ask = |token: &str, provider: airband_core::relay::PushService, to: &str| {
            let (relay, channel) = (relay.clone(), keys.channel.clone());
            let mut headers = HeaderMap::new();
            headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
            let body = serde_json::to_vec(&PushRequest { provider, to: to.into(), sealed: "c2VhbGVk".into(), question: true }).unwrap();
            async move {
                let res = wake_phone(State(relay), Path(channel), headers, Bytes::from(body)).await;
                let status = res.status().as_u16();
                let answer: PushAnswer = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap_or_default();
                (status, answer.sent, answer.gone, answer.reason)
            }
        };
        // not the channel's tower: nothing is sent
        assert_eq!(ask("someone-else", airband_core::relay::PushService::Apns, "aa11").await.0, 403);
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11").await, (200, true, false, String::new()));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "bb22").await, (410, false, true, "Unregistered".into()));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "cc33").await, (410, false, true, "BadDeviceToken".into()));
        assert_eq!(seen.load(Ordering::SeqCst), 3);
        // a service this relay does not have, and an address that is not one
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Unknown, "aa11").await.0, 503);
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11/../x").await.0, 400);
        // and only so many from one channel
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().pushed = Some((Instant::now(), push::PUSHES));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11").await.0, 429);
        assert_eq!(seen.load(Ordering::SeqCst), 3);
    }
}
